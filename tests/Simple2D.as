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
	import flash.display3D.VertexBuffer3D;
	import flash.events.Event;
	
	import flash.display3D.Context3DRenderMode;
	import flash.display.StageScaleMode;
	import flash.display.StageAlign;
	
	/**
	 * ...
	 * @author ...
	 */
	public class Simple2D extends Sprite 
	{
		private static const FLOAT_3:String = Context3DVertexBufferFormat.FLOAT_3;
		
		[Embed(source="../assets/pirate.png")]
		private static const PirateImage : Class;
		private static const pirateBMD:BitmapData = (new PirateImage()).bitmapData;
		
		private var stage3D:Stage3D;
		private var context3D:Context3D;
		
		private var indexBuffer:IndexBuffer3D;
		private var vertexBuffer:VertexBuffer3D;
		
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
			trace('context');
			context3D = stage3D.context3D;
			context3D.configureBackBuffer( stage.stageWidth, stage.stageHeight, 2, false);
			
			var assembler:AGALMiniAssembler = new AGALMiniAssembler;
			var program:Program3D = context3D.createProgram();
			
			program.upload(
				assembler.assemble(
					Context3DProgramType.VERTEX, [
						// Transform our vertices by our projection matrix and move it into temporary register
						"m44 vt0, va0, vc0",
						// Move the temporary register to out position for this vertex
						"mov op, vt0"
					].join("\n")
				),
				assembler.assemble(
					Context3DProgramType.FRAGMENT, [
						// Simply assing the fragment constant to our out color
						"mov oc, fc0"
					].join("\n")
				)
			);
			
			indexBuffer = context3D.createIndexBuffer( 3 );
			indexBuffer.uploadFromVector(new <uint>[0,1,2], 0, 3);
			
			vertexBuffer = context3D.createVertexBuffer( 3, 3 );
			vertexBuffer.uploadFromVector(new <Number>[
					-1, -1,  5,
					 1, -1,  5,
					 0,  1,  5
				], 0, 3
			);
			
			// Set vertex buffer, this is what we access in vertex shader register va0
			context3D.setVertexBufferAt(0, vertexBuffer, 0, FLOAT_3 );
			
			var projection:PerspectiveMatrix3D = new PerspectiveMatrix3D;
			projection.perspectiveFieldOfViewLH( 45 * Math.PI / 180, 1.2, 0.1, 512);
			
			// Set the projection matrix as a vertex program constant, this is what we access in vertex shader register vc0
			context3D.setProgramConstantsFromMatrix(
				Context3DProgramType.VERTEX, 0, projection, true
			);
			// Set the out color for our polygon as fragment program constant, this is what we access in fragment shader register fc0
			context3D.setProgramConstantsFromVector(
				Context3DProgramType.FRAGMENT, 0, new <Number>[1, 1, 1, 0]
			);
			
			addEventListener( Event.ENTER_FRAME, onRenderFrame );
		}
		
		private function onRenderFrame(event:Event):void 
		{
			context3D.clear(0, 0, 0, 0);
			context3D.drawTriangles(indexBuffer, 0, 1);
			context3D.present();
		}
		
	}
}
