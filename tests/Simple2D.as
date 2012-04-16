package  
{
	import com.adobe.utils.AGALMiniAssembler;
	import com.adobe.utils.PerspectiveMatrix3D;
	import flash.display.BitmapData;
	import flash.display.Sprite;
	import flash.display.Stage3D;
	import flash.display3D.Context3D;
	import flash.display3D.Context3DProgramType;
	import flash.display3D.Context3DVertexBufferFormat;
	import flash.display3D.IndexBuffer3D;
	import flash.display3D.Program3D;
	import flash.display3D.textures.Texture;
	import flash.display3D.Context3DTextureFormat;
	import flash.display3D.Context3DBlendFactor;
	import flash.display3D.VertexBuffer3D;
	import flash.events.Event;
	import flash.geom.Matrix3D;
	import flash.geom.Rectangle;
	
	
	import flash.display3D.Context3DRenderMode;
	import flash.display.StageScaleMode;
	import flash.display.StageAlign;
	
	/**
	 * ...
	 * @author ...
	 */
	public class Simple2D extends Sprite 
	{
		[Embed(source="../assets/molepeople.jpg")]
		private static const PirateImage : Class;
		private static const pirateBMD:BitmapData = new PirateImage().bitmapData;
		
		private static function makeOrthoProjection(w:Number, h:Number, n:Number, f:Number):Matrix3D
		{
			return new Matrix3D(Vector.<Number>
			([
				2/w, 0  ,       0,        0,
				0  , 2/h,       0,        0,
				0  , 0  , 1/(f-n), -n/(f-n),
				0  , 0  ,       0,        1
			]));
		}
		
		private var stage3D:Stage3D;
		private var context3D:Context3D;
		
		private var indexBuffer:IndexBuffer3D;
		private var vertexBuffer:VertexBuffer3D;
		private var uvBuffer:VertexBuffer3D;
		private var texture:Texture;
		private var program:Program3D;
		private var viewMatrix:Matrix3D;
		
		public function Simple2D() 
		{
			stage.scaleMode = StageScaleMode.NO_SCALE;
			stage.align = StageAlign.TOP_LEFT;
			
			stage3D = stage.stage3Ds[0];
			stage3D.addEventListener(Event.CONTEXT3D_CREATE, onContextCreate);
			stage3D.requestContext3D();
		}
		
		private function onContextCreate(event:Event):void 
		{
			context3D = stage3D.context3D;
			context3D.enableErrorChecking = true;
			context3D.configureBackBuffer( stage.stageWidth, stage.stageHeight, 2, false);
			
			var assembler:AGALMiniAssembler = new AGALMiniAssembler;
			program = context3D.createProgram();
			
			context3D.setProgram( program );
			program.upload(
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
					"tex ft0, v0, fs0 <2d,clamp,linear,nomip> \n"+
					"mul ft0, ft0, v0.zzzz\n" +
					"mov oc, ft0 \n"
				)
			);
			
			texture = context3D.createTexture(
				pirateBMD.width, pirateBMD.height, Context3DTextureFormat.BGRA, false
			);
			texture.uploadFromBitmapData( pirateBMD );
			
			indexBuffer = context3D.createIndexBuffer( 6 );
			vertexBuffer = context3D.createVertexBuffer( 4, 3 );
			uvBuffer = context3D.createVertexBuffer( 4, 2 );
			
			indexBuffer.uploadFromVector(new <uint>[0, 1, 2, 1, 2, 3], 0, 6);
			vertexBuffer.uploadFromVector(new <Number>[
					-100, -100, 1, // x, y, alpha
					-100, 100, 1,
					100, -100, 1,
					100, 100, 1
				], 0, 4
			);
			var rect:Rectangle = pirateBMD.rect;
			uvBuffer.uploadFromVector( new <Number>[
					0, 1,
					0, 0,
					1, 1,
					1, 0
				], 0, 4
			);
			
			// Set vertex buffer, this is what we access in vertex shader register va0
			context3D.setVertexBufferAt(0, vertexBuffer, 0, Context3DVertexBufferFormat.FLOAT_3);
			context3D.setVertexBufferAt(1, uvBuffer, 0, Context3DVertexBufferFormat.FLOAT_2);
			
			viewMatrix = makeOrthoProjection(stage.stageWidth, stage.stageHeight, 0, 100);
			
			context3D.setBlendFactors(Context3DBlendFactor.ONE, Context3DBlendFactor.ONE_MINUS_SOURCE_ALPHA);
			
			// Set the projection matrix as a vertex program constant, this is what we access in vertex shader register vc0
			context3D.setProgramConstantsFromMatrix(
				Context3DProgramType.VERTEX, 0, viewMatrix, true
			);
			
			context3D.setTextureAt(0, texture );
			
			addEventListener( Event.ENTER_FRAME, onRenderFrame );
		}
		
		private function onRenderFrame(event:Event):void 
		{
			context3D.clear(0, 0, 0, 0);
			context3D.setProgram( program );
			context3D.setBlendFactors(Context3DBlendFactor.ONE, Context3DBlendFactor.ONE_MINUS_SOURCE_ALPHA);
			context3D.setProgramConstantsFromMatrix( Context3DProgramType.VERTEX, 0, viewMatrix, true );
			context3D.setTextureAt(0, texture );
			context3D.setVertexBufferAt(0, vertexBuffer, 0, Context3DVertexBufferFormat.FLOAT_3);
			context3D.setVertexBufferAt(1, uvBuffer, 0, Context3DVertexBufferFormat.FLOAT_2);
			context3D.drawTriangles(indexBuffer, 0, 2);
			context3D.present();
		}
		
	}
}
