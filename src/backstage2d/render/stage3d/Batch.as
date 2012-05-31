package backstage2d.render.stage3d 
{
	import backstage2d.display.Actor;
	import backstage2d.display.Layer;
	import backstage2d.display.spritesheets.*;
	
	import com.adobe.utils.AGALMiniAssembler;
	
	import flash.display3D.Context3D;
	import flash.display3D.Context3DBlendFactor;
	import flash.display3D.Context3DProgramType;
	import flash.display3D.Context3DVertexBufferFormat;
	import flash.display3D.IndexBuffer3D;
	import flash.display3D.Program3D;
	import flash.display3D.VertexBuffer3D;
	import flash.geom.Rectangle;
	
	import backstage2d.namespaces.bs2d;
	use namespace bs2d;
	
	/**
	 * This is the main Batch, where the heavy lifting on the GPU is performed.
	 * Note: Giant parts of this class are based on (or copied from) example
	 * code from Julian Wixson and Aaron Charbonneau of Adobe.
	 * Thanks!!
	 * @author Kevin Newman
	 */
	final public class Batch 
	{
		public var layer:Layer;
		
		public var atlas:TextureAtlas;
		internal var vertexData:Vector.<Number> = new <Number>[];
		internal var indexData:Vector.<uint> = new <uint>[];
		internal var uvData:Vector.<Number> = new <Number>[];
		
		protected var _indexBuffer:IndexBuffer3D;
		protected var _vertexBuffer:VertexBuffer3D;
		protected var _uvBuffer:VertexBuffer3D;
		protected var shader:Program3D;
		protected var _updateVBOs:Boolean = true;
		
		internal var context:Context;
		
		public var numQuads:int// = 0;
		
		public function Batch( layer:Layer ):void
		{
			this.layer = layer;
			atlas = new TextureAtlas(512, 512);
		}
		
		protected function dispose():void {
			// :TODO:
			//System.gc();
		}
		
		public function add( actor:Actor ):void
		{
			var childVertexFirstIndex:uint;
			var rect:Rectangle;
			
			atlas.add( actor );
			
			childVertexFirstIndex = (numQuads * 12) / 3;
			vertexData.push(
				0, 0, 1, // x, y, alpha
				0, 0, 1,
				0, 0, 1,
				0, 0, 1
			);
			
			indexData.push(
				childVertexFirstIndex, childVertexFirstIndex + 1, childVertexFirstIndex + 2,
				childVertexFirstIndex, childVertexFirstIndex + 2, childVertexFirstIndex + 3
			);
			
			rect = actor.texNode.rect;
			
			uvData.push(
			/* rt */	rect.right / atlas.width,	rect.top / atlas.height, // x, y
			/* rb */	rect.right / atlas.width,	rect.bottom / atlas.height,
			/* lb */	rect.left / atlas.width,	rect.bottom / atlas.height,
			/* lt */	rect.left / atlas.width,	rect.top / atlas.height
			);
			
			numQuads = numQuads + 1;
		}
		
		/**
		 * Builds the Batch GPU assets.
		 */
		public function build():void
		{
			atlas.addMany( layer.actors );
			
			for ( var i:int = 0, n:int = layer.actors.length; i < n; ++i )
			{
				add( layer.actors[ i ] );
			}
			
			_updateVBOs = true;
		}
		
		/**
		 * Activates the Batch when a context becomes available.
		 * @param	context
		 */
		public function activate( context:Context ):void
		{
			this.context = context;
			var context3D:Context3D = context.context3D;
			var assembler:AGALMiniAssembler = new AGALMiniAssembler();
			
			shader = context3D.createProgram();
			shader.upload(
				assembler.assemble( Context3DProgramType.VERTEX,
					"dp4 op.x, va0, vc0 \n"+ // transform from stream 0 to output clipspace
					"dp4 op.y, va0, vc1 \n"+
					//"dp4 op.z, va0, vc2 \n"+
					"mov op.z, vc2.z \n"+
					"mov op.w, vc3.w \n"+
					"mov v0, va1.xy \n"+ // copy texcoord from stream 1 to fragment program
					"mov v0.z, va0.z \n" // copy alpha from stream 0 to fragment program
				),
				assembler.assemble( Context3DProgramType.FRAGMENT,
					"tex ft0, v0, fs0 <2d,clamp,linear,mipnearest> \n"+
					"mul ft0, ft0, v0.zzzz\n" +
					"mov oc, ft0 \n"
				)
			);
			
			//if (atlas.complete)
				atlas.upload( context3D );
			/*else
				atlas.made.add( function atlasMade():void {
					atlas.made.remove( atlasMade );
					atlas.upload( context3D );
				} );*/
			
			_updateVBOs = true;
		}
		
		public function draw():void
		{
			// check if new items were added to the layer
			if ( layer.actors.length > numQuads ) {
				build(); // this re-adds already added actors
				atlas.upload( context.context3D );
			}
			
			// Update vertex data with current position of children
			for ( var i:uint, n:uint = layer.actors.length; i < n; i = i + 1) {
				updateChildVertexData( layer.actors[ i ], i );
			}
			
			var _context3D:Context3D = context.context3D;
			
			_context3D.setProgram( shader );
			_context3D.setBlendFactors(Context3DBlendFactor.ONE, Context3DBlendFactor.ONE_MINUS_SOURCE_ALPHA);            
			_context3D.setProgramConstantsFromMatrix(Context3DProgramType.VERTEX, 0, context.viewMatrix, true); 
			_context3D.setTextureAt(0, atlas.texture );
			
			if ( _updateVBOs ) {
				_vertexBuffer = _context3D.createVertexBuffer( vertexData.length/3, 3);   
				_indexBuffer = _context3D.createIndexBuffer( indexData.length);
				_uvBuffer = _context3D.createVertexBuffer( uvData.length/2, 2);
				_indexBuffer.uploadFromVector( indexData, 0, indexData.length); // indices won't change                
				_uvBuffer.uploadFromVector( uvData, 0, uvData.length / 2); // child UVs won't change
				_updateVBOs = false;
			}
			
			_vertexBuffer.uploadFromVector( vertexData, 0, vertexData.length / 3);
			_context3D.setVertexBufferAt(0, _vertexBuffer, 0, Context3DVertexBufferFormat.FLOAT_3);
			_context3D.setVertexBufferAt(1, _uvBuffer, 0, Context3DVertexBufferFormat.FLOAT_2);
			
			_context3D.drawTriangles( _indexBuffer, 0,  numQuads * 2);
		}
		
		protected function updateChildVertexData( sprite:Actor, index:uint ):void
		{
			var childVertexIdx:uint = index * 12;
			
			if ( sprite.visible )
			{
				var x:Number = sprite.x;
				var y:Number = sprite.y;
				
				var sinT:Number = Math.sin(sprite.rotation);
				var cosT:Number = Math.cos(sprite.rotation);
				var alpha:Number = sprite.opacity;
				
				// use the texture width/height, which may be sized differently from the actor.
				var scaledWidth:Number = sprite.texNode.rect.width * sprite.scaleX;
				var scaledHeight:Number = sprite.texNode.rect.height * sprite.scaleY;
				
				// adjust the registration point for the texture offset and scale
				// :TODO: simplify or cache when registration.xy change somehow.
				var centerX:Number = (sprite.texNode.regOffset.x + sprite.registration.x ) * sprite.scaleX;
				var centerY:Number = (sprite.texNode.regOffset.y + sprite.registration.y ) * sprite.scaleY;
				
				vertexData[childVertexIdx] = x - (cosT * centerX) - (sinT * (scaledHeight - centerY));
				vertexData[childVertexIdx+1] = y - (sinT * centerX) + (cosT * (scaledHeight - centerY));
				vertexData[childVertexIdx+2] = alpha;
				
				vertexData[childVertexIdx+3] = x - (cosT * centerX) + (sinT * centerY);
				vertexData[childVertexIdx+4] = y - (sinT * centerX) - (cosT * centerY);
				vertexData[childVertexIdx+5] = alpha;
				
				vertexData[childVertexIdx+6] = x + (cosT * (scaledWidth - centerX)) + (sinT * centerY);
				vertexData[childVertexIdx+7] = y + (sinT * (scaledWidth - centerX)) - (cosT * centerY);
				vertexData[childVertexIdx+8] = alpha;
				
				vertexData[childVertexIdx+9] = x + (cosT * (scaledWidth - centerX)) - (sinT * (scaledHeight - centerY));
				vertexData[childVertexIdx+10] = y + (sinT * (scaledWidth - centerX)) + (cosT * (scaledHeight - centerY));
				vertexData[childVertexIdx+11] = alpha;
			}
			else {
				for (var i:uint = 0; i < 12; i++ ) {
					vertexData[childVertexIdx+i] = 0;
				}
			}
		}
	}
}
