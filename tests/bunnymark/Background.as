/*
========================================================================
=      ================================  =====  ==================  ====
=  ===  ===============================   ===   ==================  ====
=  ====  ==============================  =   =  ==================  ====
=  ===  ===  =  ==  = ===  = ===  =  ==  == ==  ===   ===  =   ===  =  =
=      ====  =  ==     ==     ==  =  ==  =====  ==  =  ==    =  ==    ==
=  ===  ===  =  ==  =  ==  =  ===    ==  =====  =====  ==  =======   ===
=  ====  ==  =  ==  =  ==  =  =====  ==  =====  ===    ==  =======    ==
=  ===  ===  =  ==  =  ==  =  ==  =  ==  =====  ==  =  ==  =======  =  =
=      =====    ==  =  ==  =  ===   ===  =====  ===    ==  =======  =  =
========================================================================


* Copyright (c) 2012 Julian Wixson / Aaron Charbonneau - Adobe Systems
*
* Special thanks to Iain Lobb - iainlobb@googlemail.com for the original BunnyMark:
*
* http://blog.iainlobb.com/2010/11/display-list-vs-blitting-results.html 
*
* This program is distributed under the terms of the MIT License as found 
* in a file called LICENSE. If it is not present, the license
* is always available at http://www.opensource.org/licenses/mit-license.php.
*
* This program is distributed in the hope that it will be useful, but
* without any waranty; without even the implied warranty of merchantability
* or fitness for a particular purpose. See the MIT License for full details.
*/

package bunnymark
{
	import com.adobe.utils.AGALMiniAssembler;
	
	import flash.display.Bitmap;
	import flash.display.BitmapData;
	import flash.display3D.*;
	import flash.display3D.textures.Texture;
	import flash.geom.Matrix3D;
	import flash.utils.getTimer;
	import flash.geom.Rectangle;
	import flash.geom.Matrix;
	
	public class Background
	{
		[Embed(source="../../assets/grass.png")]
		static private const Grass : Class;
		private var context3D:Context3D;
		private var vb:VertexBuffer3D;
		private var uvb:VertexBuffer3D;
		private var ib:IndexBuffer3D;
		private var shader_program:Program3D;
		private var tex:Texture;
		private var _width:Number;
		private var _height:Number;
		static private const texBM:Bitmap = new Grass();
		private var _modelViewMatrix : Matrix3D;
		
		//variables for vertexBuffer manipulation
		private var vertices:Vector.<Number>;
		private var indices:Vector.<uint>;
		private var uvt:Vector.<Number>;
		
		//haxe variable
		public var cols:int = 8;
		public var rows:int = 12;
		public var numTriangles:int;
		public var numVertices:int;
		public var numIndices:int;
		
		public function Background( ctx3D:Context3D, w:Number, h:Number )
		{
			context3D = ctx3D;
		}
		
		public function setup():void
		{
			//create background texture
			tex = context3D.createTexture(texBM.width, texBM.height, Context3DTextureFormat.BGRA,false);
			tex.uploadFromBitmapData(texBM.bitmapData,0);
			
			
			// Courtesy of Starling: let's generate mipmaps
            var currentWidth:int = texBM.width >> 1;
            var currentHeight:int = texBM.height >> 1;
            var level:int = 1;
            var canvas:BitmapData = new BitmapData(currentWidth, currentHeight, true, 0);
            var transform:Matrix = new Matrix(.5, 0, 0, .5);
            
            while ( currentWidth >= 1 || currentHeight >= 1 ) {
                canvas.fillRect(new Rectangle(0, 0, Math.max(currentWidth,1), Math.max(currentHeight,1)), 0);
                canvas.draw(texBM, transform, null, null, null, true);
                tex.uploadFromBitmapData(canvas, level++);
                transform.scale(0.5, 0.5);
                currentWidth = currentWidth >> 1;
                currentHeight = currentHeight >> 1;
            }
			
			
			//create vertices
			buildMesh();
			
			//build shaders
			var miniasm_vertex : AGALMiniAssembler = new AGALMiniAssembler ();
			miniasm_vertex.assemble( Context3DProgramType.VERTEX,
				"m44 op, va0, vc0  \n" +        // 4x4 matrix transform to output clipspace
				"mov v0, va1       \n");        // pass texture coordinates to fragment program	
			var miniasm_fragment : AGALMiniAssembler = new AGALMiniAssembler (); 
			miniasm_fragment.assemble(Context3DProgramType.FRAGMENT,
				"tex oc, v0, fs0 <2d,linear,miplinear,wrap>" );// sample texture 0
			shader_program = context3D.createProgram();
			shader_program.upload( miniasm_vertex.agalcode, miniasm_fragment.agalcode );		
			
			//create projection matrix
			_modelViewMatrix = new Matrix3D();
			_modelViewMatrix.appendTranslation(-(_width)/2, -(_height)/2, 0);            
			_modelViewMatrix.appendScale(2.0/(_width-50), -2.0/(_height-50), 1);
			
			//set everything
			context3D.setTextureAt(0,tex);  
			context3D.setProgram ( shader_program );
			context3D.setVertexBufferAt( 0, vb, 0, "float2" );  
			context3D.setVertexBufferAt( 1, uvb, 0, "float2" );
			context3D.setProgramConstantsFromMatrix(Context3DProgramType.VERTEX, 0,_modelViewMatrix,true);
			
		}

		public function setPosition(view:Rectangle):void {
			_width = view.width;
			_height = view.height;
			//recreate the mesh coords
			buildMesh();
			//resize the projection
			_modelViewMatrix = new Matrix3D();
			_modelViewMatrix.appendTranslation(-(_width)/2, -(_height)/2, 0);            
			_modelViewMatrix.appendScale(2.0/(_width-50), -2.0/(_height-50), 1);
		}
		private function buildMesh():void 
		{
			var uw:Number = _width / texBM.width;
			var uh:Number = _height / texBM.height;
			var kx:Number, ky:Number;
			var ci:int, ci2:int, ri:int;
			
			vertices = new Vector.<Number>();
			uvt = new Vector.<Number>();
			indices = new Vector.<uint>();
			
			var i:int;
			var j:int;
			for(j = 0; j <= rows; j++)
			{
				ri = j * (cols + 1) * 2;
				ky = j / rows;
				for(i = 0; i <= cols; i++)
				{
					ci = ri + i * 2;
					kx = i / cols;
					vertices[ci] = _width * kx; 
					vertices[ci + 1] = _height * ky;
					uvt[ci] = uw * kx; 
					uvt[ci + 1] = uh * ky;
				}
			}
			for(j = 0; j < rows; j++)
			{
				ri = j * (cols + 1);
				for(i = 0; i < cols; i++)
				{
					ci = i + ri;
					ci2 = ci + cols + 1;
					indices.push(ci);
					indices.push(ci + 1);
					indices.push(ci2);
					indices.push(ci + 1);
					indices.push(ci2 + 1);
					indices.push(ci2);
				}
			}
			//now create the buffers
			numIndices = indices.length;
			numTriangles = numIndices / 3;
			numVertices = vertices.length / 2;
	
			vb = context3D.createVertexBuffer(numVertices,2);
			uvb = context3D.createVertexBuffer(numVertices,2);
			
			ib = context3D.createIndexBuffer(numIndices);
			vb.uploadFromVector(vertices,0,numVertices);
			ib.uploadFromVector(indices,0,numIndices);
			uvb.uploadFromVector(uvt,0,numVertices);
			
		}
		public function render():void {
			if (_width == 0 || _height == 0) return;
			
			var t:Number = getTimer() / 1000.0;
			var sw:Number = _width;
			var sh:Number = _height;
			var kx:Number, ky:Number;
			var ci:int, ri:int;
			context3D.setBlendFactors(Context3DBlendFactor.ONE, Context3DBlendFactor.ONE_MINUS_SOURCE_ALPHA);
			context3D.setTextureAt(0,tex);  
			context3D.setProgram ( shader_program );
			context3D.setVertexBufferAt( 0, vb, 0, "float2" );  
			context3D.setVertexBufferAt( 1, uvb, 0, "float2" );
			context3D.setProgramConstantsFromMatrix(Context3DProgramType.VERTEX, 0,_modelViewMatrix,true);
			
			var i:int = 0;
			for(var j:int = 0; j <= rows; j++)
			{
				ri = j * (cols + 1) * 2;
				for (i=0; i <= cols; i++) 
				{
					ci = ri + i * 2;
					kx = i / cols + Math.cos(t + i) * 0.02;
					ky = j / rows + Math.sin(t + j + i) * 0.02;
					vertices[ci] = sw * kx; 
					vertices[ci + 1] = sh * ky; 
				}
			}
			vb.uploadFromVector(vertices,0,numVertices);
			context3D.drawTriangles(ib,0,numTriangles);
		}
	}
}